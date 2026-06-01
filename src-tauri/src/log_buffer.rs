//! In-process log ring buffer with a runtime level toggle.
//!
//! The Tauri webview is the only practical place to inspect logs on
//! macOS bundled apps — there's no terminal attached to a .app launch.
//! So we install a logger that keeps the last N records in memory and
//! exposes them via Tauri commands (see commands/logs.rs).
//!
//! Design:
//! - `log::Log` impl writes to a bounded VecDeque (default 2000 entries,
//!   FIFO eviction).
//! - Also forwards to stderr in env_logger-style formatting so a
//!   developer running `cargo run` still sees output live.
//! - `log::set_max_level` controls the verbosity at runtime — the UI
//!   flips this from the Help → Logs panel without restarting.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use log::{Level, LevelFilter, Log, Metadata, Record};
use serde::{Deserialize, Serialize};

const DEFAULT_CAPACITY: usize = 2000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    /// Milliseconds since Unix epoch. Unsigned because we never log
    /// from before 1970.
    pub ts_ms: u64,
    /// "error" | "warn" | "info" | "debug" | "trace".
    pub level: String,
    /// `log::Record::target()` — typically a module path like
    /// "kraftwerk_lib::libvirt::connection".
    pub target: String,
    pub message: String,
}

#[derive(Clone)]
pub struct LogBuffer {
    inner: Arc<Mutex<VecDeque<LogEntry>>>,
    capacity: usize,
}

impl LogBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(VecDeque::with_capacity(capacity.min(8192)))),
            capacity,
        }
    }

    fn push(&self, e: LogEntry) {
        let mut g = match self.inner.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(), // mutex poisoned in another thread — keep going
        };
        if g.len() >= self.capacity {
            g.pop_front();
        }
        g.push_back(e);
    }

    /// Snapshot of every entry currently buffered (oldest first).
    pub fn snapshot(&self) -> Vec<LogEntry> {
        let g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        g.iter().cloned().collect()
    }

    /// Subset newer than `after_ts_ms` (exclusive), optionally filtered
    /// by minimum severity (entries at or above this level pass).
    pub fn filtered(&self, after_ts_ms: Option<u64>, min_level: Option<Level>) -> Vec<LogEntry> {
        let g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        g.iter()
            .filter(|e| match after_ts_ms { Some(t) => e.ts_ms > t, None => true })
            .filter(|e| match min_level {
                Some(min) => level_from_str(&e.level)
                    .map(|l| l <= min)  // Level: Error < Warn < Info < Debug < Trace
                    .unwrap_or(true),
                None => true,
            })
            .cloned()
            .collect()
    }

    pub fn clear(&self) {
        let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        g.clear();
    }
}

/// The actual `log::Log` impl. One per process — installed via
/// `init_logger`.
struct BufferedLogger {
    buffer: LogBuffer,
}

impl Log for BufferedLogger {
    fn enabled(&self, m: &Metadata) -> bool {
        m.level() <= log::max_level()
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let ts_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let message = format!("{}", record.args());
        // Best-effort stderr passthrough so `cargo run` still shows logs.
        eprintln!(
            "[{}] {:<5} {}: {}",
            ts_ms,
            record.level(),
            record.target(),
            message,
        );
        self.buffer.push(LogEntry {
            ts_ms,
            level: record.level().to_string().to_lowercase(),
            target: record.target().to_string(),
            message,
        });
    }

    fn flush(&self) {}
}

/// Install the buffered logger as the global `log` sink. Safe to call
/// multiple times — extra calls are no-ops (first call wins, mimicking
/// env_logger's behaviour).
pub fn init_logger(buffer: LogBuffer, initial_level: LevelFilter) -> LogBuffer {
    let logger = Box::new(BufferedLogger { buffer: buffer.clone() });
    // log::set_boxed_logger can only succeed once per process. Tests
    // and re-init paths get a no-op — the existing logger keeps the
    // buffer they reference because we hand out the LogBuffer Arc.
    let _ = log::set_boxed_logger(logger);
    log::set_max_level(initial_level);
    buffer
}

/// Default buffer + initial Info level. Convenience for `run()`.
pub fn default_init() -> LogBuffer {
    let buf = LogBuffer::new(DEFAULT_CAPACITY);
    init_logger(buf.clone(), LevelFilter::Info)
}

/// Parse a UI-side level string into a `log::LevelFilter`. Accepts
/// "off" | "error" | "warn" | "info" | "debug" | "trace" case-insensitively.
pub fn level_filter_from_str(s: &str) -> Option<LevelFilter> {
    match s.to_ascii_lowercase().as_str() {
        "off" => Some(LevelFilter::Off),
        "error" => Some(LevelFilter::Error),
        "warn" => Some(LevelFilter::Warn),
        "info" => Some(LevelFilter::Info),
        "debug" => Some(LevelFilter::Debug),
        "trace" => Some(LevelFilter::Trace),
        _ => None,
    }
}

fn level_from_str(s: &str) -> Option<Level> {
    match s.to_ascii_lowercase().as_str() {
        "error" => Some(Level::Error),
        "warn" => Some(Level::Warn),
        "info" => Some(Level::Info),
        "debug" => Some(Level::Debug),
        "trace" => Some(Level::Trace),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_buffer_evicts_oldest() {
        let b = LogBuffer::new(3);
        for i in 0..5 {
            b.push(LogEntry { ts_ms: i, level: "info".into(), target: "t".into(), message: format!("m{i}") });
        }
        let s = b.snapshot();
        assert_eq!(s.len(), 3);
        assert_eq!(s[0].ts_ms, 2);
        assert_eq!(s[2].ts_ms, 4);
    }

    #[test]
    fn filter_by_ts() {
        let b = LogBuffer::new(10);
        for i in 1..=5 {
            b.push(LogEntry { ts_ms: i, level: "info".into(), target: "t".into(), message: "x".into() });
        }
        assert_eq!(b.filtered(Some(2), None).len(), 3);
        assert_eq!(b.filtered(None, None).len(), 5);
        assert_eq!(b.filtered(Some(5), None).len(), 0);
    }

    #[test]
    fn filter_by_min_level() {
        let b = LogBuffer::new(10);
        b.push(LogEntry { ts_ms: 1, level: "error".into(), target: "t".into(), message: "e".into() });
        b.push(LogEntry { ts_ms: 2, level: "warn".into(),  target: "t".into(), message: "w".into() });
        b.push(LogEntry { ts_ms: 3, level: "info".into(),  target: "t".into(), message: "i".into() });
        b.push(LogEntry { ts_ms: 4, level: "debug".into(), target: "t".into(), message: "d".into() });
        // min_level = Warn → only Error + Warn pass.
        let got = b.filtered(None, Some(Level::Warn));
        assert_eq!(got.len(), 2);
        let levels: Vec<_> = got.iter().map(|e| e.level.as_str()).collect();
        assert!(levels.contains(&"error"));
        assert!(levels.contains(&"warn"));
    }

    #[test]
    fn level_filter_parse() {
        assert_eq!(level_filter_from_str("off"), Some(LevelFilter::Off));
        assert_eq!(level_filter_from_str("Info"), Some(LevelFilter::Info));
        assert_eq!(level_filter_from_str("DEBUG"), Some(LevelFilter::Debug));
        assert_eq!(level_filter_from_str("nope"), None);
    }

    #[test]
    fn clear_empties_the_buffer() {
        let b = LogBuffer::new(10);
        b.push(LogEntry { ts_ms: 1, level: "info".into(), target: "t".into(), message: "x".into() });
        assert_eq!(b.snapshot().len(), 1);
        b.clear();
        assert_eq!(b.snapshot().len(), 0);
    }
}
