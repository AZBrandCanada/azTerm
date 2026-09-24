// src/debug_log.rs
//
// Lightweight, opt-in debug logging with a panic hook.
//
// When `debug_mode` is on in Settings, key operations write timestamped
// lines to the configured log file. If the app ever hangs, the last line
// in the log tells you which operation was running when it stopped
// responding. If the app panics, the panic message and a full backtrace
// are appended here too.
//
// The `is_enabled()` check is a lock-free atomic so hot-path callers pay
// essentially nothing when debug mode is off (the default).

use std::fs::OpenOptions;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

const MAX_LOG_BYTES: u64 = 20 * 1024 * 1024; // rotate at 20 MB

struct LoggerState {
    writer: Option<BufWriter<std::fs::File>>,
    path: String,
}

static ENABLED: AtomicBool = AtomicBool::new(false);
static LOGGER: OnceLock<Mutex<LoggerState>> = OnceLock::new();

fn state() -> &'static Mutex<LoggerState> {
    LOGGER.get_or_init(|| {
        Mutex::new(LoggerState {
            writer: None,
            path: String::new(),
        })
    })
}

/// Called at startup and whenever the setting changes.
pub fn init(enabled: bool, path: &str) {
    ENABLED.store(enabled, Ordering::Relaxed);

    let mut s = match state().lock() {
        Ok(s) => s,
        Err(_) => return,
    };

    // If path changed, close the current writer.
    if path != s.path {
        s.writer = None;
        s.path = path.to_string();
    }

    if !enabled || path.is_empty() {
        return;
    }

    if s.writer.is_none() {
        if let Some(parent) = Path::new(path).parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        // Rotate if oversized. Preserves the previous run.
        if let Ok(meta) = std::fs::metadata(path) {
            if meta.len() > MAX_LOG_BYTES {
                let rotated = format!("{}.old", path);
                let _ = std::fs::rename(path, &rotated);
            }
        }

        match OpenOptions::new().create(true).append(true).open(path) {
            Ok(f) => {
                let mut w = BufWriter::new(f);
                let stamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
                let _ = writeln!(
                    w,
                    "\n===== AZTerm v{} debug session started {} =====",
                    env!("CARGO_PKG_VERSION"),
                    stamp,
                );
                let _ = w.flush();
                s.writer = Some(w);
            }
            Err(e) => {
                eprintln!("[debug_log] could not open {}: {}", path, e);
            }
        }
    }
}

#[inline]
pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

pub fn log(msg: impl AsRef<str>) {
    if !is_enabled() {
        return;
    }
    let mut s = match state().lock() {
        Ok(s) => s,
        Err(_) => return,
    };
    if let Some(w) = s.writer.as_mut() {
        let now = chrono::Local::now().format("%H:%M:%S%.3f");
        let _ = writeln!(w, "[{}] {}", now, msg.as_ref());
        let _ = w.flush();
    }
}

/// Install a panic hook that records the panic message + backtrace to
/// the debug log before delegating to the default hook (so stderr still
/// shows it, and RUST_BACKTRACE still works).
pub fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "<unknown>".to_string());

        let payload = if let Some(s) = info.payload().downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "<non-string panic payload>".to_string()
        };

        let thread = std::thread::current();
        let name = thread.name().unwrap_or("<unnamed>");

        let bt = std::backtrace::Backtrace::force_capture();
        log(format!(
            "PANIC thread='{}' at {}: {}\nBacktrace:\n{}",
            name, location, payload, bt
        ));

        default_hook(info);
    }));
}

/// Log a formatted message when debug mode is enabled.
#[macro_export]
macro_rules! dbg_log {
    ($($arg:tt)*) => {
        if $crate::debug_log::is_enabled() {
            $crate::debug_log::log(format!($($arg)*));
        }
    };
}
