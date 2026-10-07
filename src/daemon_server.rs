// src/bin/azterm_daemon.rs
//
// Background process that owns AZTerm PTYs. Keeps shells, SSH
// connections, and running commands alive when the GUI window closes.
//
// Spawned automatically on first launch when `use_daemon = true` in
// Settings → Terminal Interaction. Refuses to start a second copy if
// one is already listening on the socket.
//
// Protocol lives in src/daemon.rs, shared with the GUI via #[path].

use crate::daemon::{b64_decode, b64_encode, read_msg, write_msg, Request, Response, SessionInfo, socket_path, PROTO_VERSION};
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

const REPLAY_LIMIT: usize = 512 * 1024;

struct DaemonSession {
    id: u64,
    title: String,
    kind: String,
    target: String,
    cols: u16,
    rows: u16,
    child_pid: Option<u32>,
    master_pty: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    writer_tx: SyncSender<Vec<u8>>,
    replay: Arc<Mutex<VecDeque<u8>>>,
    subscribers: Arc<Mutex<Vec<SyncSender<Vec<u8>>>>>,
    alive: Arc<Mutex<bool>>,
}

impl DaemonSession {
    fn info(&self) -> SessionInfo {
        SessionInfo {
            id: self.id,
            title: self.title.clone(),
            kind: self.kind.clone(),
            target: self.target.clone(),
            cols: self.cols,
            rows: self.rows,
            alive: *self.alive.lock().unwrap_or_else(|e| e.into_inner()),
        }
    }
    fn resize(&self, cols: u16, rows: u16) {
        if let Ok(m) = self.master_pty.lock() {
            let _ = m.resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            });
        }
    }
    fn kill_child(&self) {
        #[cfg(unix)]
        if let Some(pid) = self.child_pid {
            unsafe {
                libc::kill(pid as i32, libc::SIGHUP);
            }
        }
    }
}

type Shared = Arc<Mutex<HashMap<u64, Arc<DaemonSession>>>>;

pub fn run() {
    let path = socket_path();
    let lock_path = path.with_extension("lock");

    // ---- Kernel-level singleton guard -------------------------------
    //
    // Hold an exclusive flock on a sibling lock file for the process
    // lifetime. The kernel releases it automatically when we exit for
    // ANY reason (clean return, panic, SIGKILL), so a stale lock file
    // on disk is never itself a problem — only a LIVE holder can block
    // us.
    //
    // Without this, two GUIs racing at startup could each spawn a
    // daemon: the second one would find the first's socket, fail to
    // ping it within the 2-second window (busy reading PTY output),
    // conclude it was stale, unlink it, and bind its own. The first
    // daemon's PTYs would become permanently unreachable — exactly the
    // "session doesn't come back" symptom.
    let _lock_file = match acquire_daemon_lock(&lock_path) {
        Some(f) => f,
        None => {
            eprintln!("azterm-daemon: another instance holds the lock, exiting");
            return;
        }
    };

    // Safe to remove any pre-existing socket: we hold the lock, so no
    // live daemon owns it. If the previous holder died without cleaning
    // up, its socket file is a harmless orphaned inode at this point.
    let _ = std::fs::remove_file(&path);

    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("azterm-daemon: bind failed: {}", e);
            std::process::exit(1);
        }
    };
    eprintln!("azterm-daemon: listening on {}", path.display());

    let shared: Shared = Arc::new(Mutex::new(HashMap::new()));

    for stream in listener.incoming() {
        if let Ok(s) = stream {
            let sh = shared.clone();
            thread::spawn(move || {
                let _ = handle_client(s, sh);
            });
        }
    }
}

/// Try to acquire the daemon singleton lock. On success returns the
/// held File (which MUST be kept alive for the process lifetime — the
/// lock is released when it drops). Returns None if another process
/// currently holds the lock.
fn acquire_daemon_lock(lock_path: &std::path::Path) -> Option<std::fs::File> {
    use std::os::unix::io::AsRawFd;
    let f = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(lock_path)
        .ok()?;
    let rc = unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc == 0 {
        Some(f)
    } else {
        None
    }
}

fn handle_client(mut stream: UnixStream, shared: Shared) -> std::io::Result<()> {
    loop {
        let req: Request = match read_msg(&mut stream) {
            Ok(r) => r,
            Err(_) => return Ok(()),
        };

        match req {
            Request::Ping => write_msg(
                &mut stream,
                &Response::Pong {
                    proto_version: PROTO_VERSION,
                },
            )?,

            Request::List => {
                let sessions: Vec<SessionInfo> = {
                    let sh = shared.lock().unwrap();
                    sh.values().map(|s| s.info()).collect()
                };
                write_msg(&mut stream, &Response::Sessions(sessions))?;
            }

            Request::NewLocal { id, title, cwd, shell, cols, rows } => {
                let r = create_local(id, title, cwd, shell, cols, rows, shared.clone());
                write_msg(&mut stream, &result_resp(r))?;
            }

            Request::NewSsh { id, title, spec, cols, rows } => {
                let r = create_ssh(id, title, spec, cols, rows, shared.clone());
                write_msg(&mut stream, &result_resp(r))?;
            }

            Request::Kill { id } => {
                let removed = {
                    let mut sh = shared.lock().unwrap();
                    sh.remove(&id)
                };
                if let Some(s) = removed {
                    *s.alive.lock().unwrap() = false;
                    s.kill_child();
                }
                write_msg(&mut stream, &Response::Ok)?;
            }

            Request::Shutdown => {
                // Fire-and-forget from the GUI: kill every child,
                // unlink the socket so a later try_connect() sees
                // "no daemon", then exit. The kernel releases the
                // singleton flock on exit, so a fresh daemon can
                // start immediately if the user toggles the setting
                // back on.
                let victims: Vec<_> = {
                    let mut sh = shared.lock().unwrap();
                    sh.drain().map(|(_, s)| s).collect()
                };
                for s in &victims {
                    *s.alive.lock().unwrap() = false;
                    s.kill_child();
                }
                let _ = std::fs::remove_file(crate::daemon::socket_path());
                std::process::exit(0);
            }

            Request::Attach { id, cols, rows } => {
                let sess = {
                    let sh = shared.lock().unwrap();
                    sh.get(&id).cloned()
                };
                let sess = match sess {
                    Some(s) => s,
                    None => {
                        write_msg(&mut stream, &Response::Error("session not found".into()))?;
                        continue;
                    }
                };
                sess.resize(cols, rows);
                write_msg(&mut stream, &Response::Attached { id })?;

                // Replay scrollback first.
                let replay_bytes: Vec<u8> = {
                    let r = sess.replay.lock().unwrap();
                    r.iter().copied().collect()
                };
                if !replay_bytes.is_empty() {
                    write_msg(
                        &mut stream,
                        &Response::Output {
                            bytes_b64: b64_encode(&replay_bytes),
                        },
                    )?;
                }

                // If the child process already exited while this client
                // was disconnected (e.g. SSH timed out, user typed
                // 'exit', server dropped the connection), do NOT enter
                // the streaming loop. The reader thread that would feed
                // subscribers has already exited, so the loop would hang
                // forever and the client would never learn the session
                // is dead. Send Closed immediately so the GUI flips the
                // tile to "session ended" and shows the Reconnect
                // overlay.
                let alive_now =
                    *sess.alive.lock().unwrap_or_else(|e| e.into_inner());
                if !alive_now {
                    write_msg(&mut stream, &Response::Closed)?;
                    return Ok(());
                }

                stream_output(stream, sess)?;
                return Ok(());
            }

            _ => {
                write_msg(&mut stream, &Response::Error("unexpected request".into()))?;
            }
        }
    }
}

fn result_resp(r: Result<(), String>) -> Response {
    match r {
        Ok(()) => Response::Ok,
        Err(e) => Response::Error(e),
    }
}

fn stream_output(stream: UnixStream, sess: Arc<DaemonSession>) -> std::io::Result<()> {
    let read_stream = stream.try_clone()?;
    let write_stream = Arc::new(Mutex::new(stream));
    let shutdown = Arc::new(AtomicBool::new(false));

    let (tx, rx): (SyncSender<Vec<u8>>, Receiver<Vec<u8>>) = sync_channel(1024);
    sess.subscribers.lock().unwrap().push(tx);

    // Writer thread — forwards broadcast output to this client.
    let ws = write_stream.clone();
    let sd = shutdown.clone();
    let w_thread = thread::spawn(move || {
        while !sd.load(Ordering::Relaxed) {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(bytes) => {
                    let mut w = ws.lock().unwrap();
                    if write_msg(
                        &mut *w,
                        &Response::Output {
                            bytes_b64: b64_encode(&bytes),
                        },
                    )
                    .is_err()
                    {
                        break;
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        if let Ok(mut w) = ws.lock() {
            let _ = write_msg(&mut *w, &Response::Closed);
        }
    });

    // Reader loop — Input + Resize from this client.
    let mut reader = read_stream;
    loop {
        let req: Request = match read_msg(&mut reader) {
            Ok(r) => r,
            Err(_) => break,
        };
        match req {
            Request::Input { bytes_b64 } => {
                let bytes = b64_decode(&bytes_b64);
                if !bytes.is_empty() {
                    // BLOCKING send. Do not change this back to
                    // try_send().
                    //
                    // The channel to the PTY writer is bounded. If
                    // it is full — which happens the moment nano,
                    // vim, or any TUI is slow to drain its stdin —
                    // try_send() returns Err(Full) and the keystroke
                    // is silently thrown away. From the user's side
                    // the cursor "moves less than I pressed", or a
                    // whole sequence of arrows is dropped, and
                    // eventually they are editing at a different
                    // position than the display shows.
                    //
                    // Blocking here backpressures the client's
                    // writer thread, which backpressures the UI's
                    // input channel. Input is never lost. The
                    // observed effect during a slow redraw is a
                    // momentary lag, which is correct.
                    let _ = sess.writer_tx.send(bytes);
                }
            }
            Request::Resize { cols, rows } => {
                sess.resize(cols, rows);
            }
            _ => {}
        }
    }

    shutdown.store(true, Ordering::Relaxed);
    drop(sess);
    let _ = w_thread.join();
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn spawn_session_workers(
    id: u64,
    title: String,
    kind: String,
    target: String,
    cols: u16,
    rows: u16,
    child_pid: Option<u32>,
    master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    mut reader: Box<dyn Read + Send>,
    mut writer: Box<dyn Write + Send>,
    shared: Shared,
) {
    // Large input buffer so a momentarily slow PTY write does not
    // cause try_send (in handle_client's Request::Input branch) to
    // silently drop keystrokes.
    let (writer_tx, writer_rx): (SyncSender<Vec<u8>>, Receiver<Vec<u8>>) = sync_channel(65536);

    thread::spawn(move || {
        while let Ok(bytes) = writer_rx.recv() {
            if writer.write_all(&bytes).is_err() {
                break;
            }
            let _ = writer.flush();
        }
    });

    let replay = Arc::new(Mutex::new(VecDeque::<u8>::new()));
    let subscribers = Arc::new(Mutex::new(Vec::<SyncSender<Vec<u8>>>::new()));
    let alive = Arc::new(Mutex::new(true));

    let sess = Arc::new(DaemonSession {
        id,
        title,
        kind,
        target,
        cols,
        rows,
        child_pid,
        master_pty: master,
        writer_tx,
        replay: replay.clone(),
        subscribers: subscribers.clone(),
        alive: alive.clone(),
    });

    let replay_r = replay.clone();
    let subs_r = subscribers.clone();
    let alive_r = alive.clone();

    thread::spawn(move || {
        let mut buf = [0u8; 16384];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let bytes = buf[..n].to_vec();
                    {
                        let mut r = replay_r.lock().unwrap();
                        r.extend(bytes.iter().copied());
                        while r.len() > REPLAY_LIMIT {
                            r.pop_front();
                        }
                    }
                    // BLOCKING broadcast. Do not change this back to
                    // try_send().
                    //
                    // The subscriber channel is bounded. try_send()
                    // silently DROPS the chunk when the buffer is
                    // full, which punches a hole in the byte stream
                    // the client feeds to its vt100 parser. If that
                    // hole lands mid-escape-sequence, the parser's
                    // cursor state silently diverges from the app's
                    // real cursor — the "cursor loses its spot in
                    // nano and edits land on the wrong row" bug.
                    //
                    // Blocking instead backpressures the PTY reader,
                    // which backpressures the child process (nano,
                    // vim, htop...). Slightly slower, never wrong.
                    //
                    // We hold subs_r across the send; that also
                    // delays new Attach requests while a slow
                    // subscriber is being backpressured, which is
                    // acceptable — the alternative (snapshot,
                    // unlock, send, reacquire) would need sender
                    // tagging to avoid losing subscribers that
                    // attached mid-broadcast.
                    let mut subs = subs_r.lock().unwrap();
                    subs.retain(|tx| tx.send(bytes.clone()).is_ok());
                }
                Err(_) => break,
            }
        }
        *alive_r.lock().unwrap() = false;
        let end: &[u8] = b"\r\n\x1b[2m[azterm: session ended]\x1b[0m\r\n";
        let mut subs = subs_r.lock().unwrap();
        for tx in subs.iter() {
            let _ = tx.try_send(end.to_vec());
        }
        subs.clear();
    });

    shared.lock().unwrap().insert(id, sess);
}

fn create_local(
    id: u64,
    title: String,
    cwd: Option<String>,
    shell: String,
    cols: u16,
    rows: u16,
    shared: Shared,
) -> Result<(), String> {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| e.to_string())?;

    let mut cmd = CommandBuilder::new(&shell);
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    cmd.env_remove("LINES");
    cmd.env_remove("COLUMNS");
    if let Ok(lang) = std::env::var("LANG") {
        cmd.env("LANG", lang);
    }

    let work_dir = cwd.unwrap_or_else(|| std::env::var("HOME").unwrap_or_else(|_| ".".into()));
    cmd.cwd(&work_dir);

    let child = pair.slave.spawn_command(cmd).map_err(|e| e.to_string())?;
    let child_pid = child.process_id();

    // Reaper thread: portable-pty's Child handle must be wait()ed on,
    // or the kernel keeps a zombie entry for every shell/SSH the
    // daemon has ever spawned. We hand the handle to a thread that
    // blocks until the child exits — whether that's a normal shell
    // exit, EOF on the PTY, or SIGHUP from DaemonSession::kill_child
    // — then reaps it.
    std::thread::spawn(move || {
        let mut child = child;
        let _ = child.wait();
    });

    let reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
    let writer = pair.master.take_writer().map_err(|e| e.to_string())?;
    let master = Arc::new(Mutex::new(pair.master));

    spawn_session_workers(
        id,
        title,
        "local".into(),
        work_dir,
        cols,
        rows,
        child_pid,
        master,
        Box::new(reader),
        Box::new(writer),
        shared,
    );
    Ok(())
}

fn create_ssh(
    id: u64,
    title: String,
    spec: crate::daemon::SshSpec,
    cols: u16,
    rows: u16,
    shared: Shared,
) -> Result<(), String> {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| e.to_string())?;

    let mut cmd = CommandBuilder::new("ssh");
    // portable_pty::CommandBuilder::arg returns (), not &mut Self, so
    // each flag/value pair has to be a separate statement.
    cmd.arg("-o");
    cmd.arg("ControlMaster=auto");
    if let Some(ref cp) = spec.control_path {
        cmd.arg("-o");
        cmd.arg(format!("ControlPath={}", cp));
        cmd.arg("-o");
        cmd.arg("ControlPersist=5m");
    }
    cmd.arg("-o");
    cmd.arg("ServerAliveInterval=15");
    cmd.arg("-o");
    cmd.arg("ServerAliveCountMax=3");
    cmd.arg("-o");
    cmd.arg("ConnectTimeout=10");
    cmd.arg("-o");
    cmd.arg("TCPKeepAlive=yes");
    cmd.arg("-p");
    cmd.arg(spec.port.to_string());
    if let Some(ref idf) = spec.identity_file {
        if !idf.trim().is_empty() {
            cmd.arg("-i");
            cmd.arg(idf.trim());
        }
    }
    cmd.arg(format!("{}@{}", spec.username, spec.host));
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");

    let child = pair.slave.spawn_command(cmd).map_err(|e| e.to_string())?;
    let child_pid = child.process_id();

    // Reaper thread: portable-pty's Child handle must be wait()ed on,
    // or the kernel keeps a zombie entry for every shell/SSH the
    // daemon has ever spawned. We hand the handle to a thread that
    // blocks until the child exits — whether that's a normal shell
    // exit, EOF on the PTY, or SIGHUP from DaemonSession::kill_child
    // — then reaps it.
    std::thread::spawn(move || {
        let mut child = child;
        let _ = child.wait();
    });

    let reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
    let writer = pair.master.take_writer().map_err(|e| e.to_string())?;
    let master = Arc::new(Mutex::new(pair.master));

    // Prefer the profile id the GUI supplied; fall back to
    // user@host:port for old clients. The GUI reads this back on
    // reattach and needs the real profile id (not the connection
    // string) to re-target SFTP path sync.
    let target = spec
        .profile_id
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("{}@{}:{}", spec.username, spec.host, spec.port));

    spawn_session_workers(
        id,
        title,
        "ssh".into(),
        target,
        cols,
        rows,
        child_pid,
        master,
        Box::new(reader),
        Box::new(writer),
        shared,
    );
    Ok(())
}
