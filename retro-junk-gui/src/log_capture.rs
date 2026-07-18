//! Custom log capture that stores log entries in a ring buffer while
//! delegating to `env_logger` for stderr output.
//!
//! Usage: call `init()` once at startup instead of `env_logger::init()`.
//! All `log::*` macros will then be captured automatically.

#[cfg(test)]
#[path = "log_capture_tests.rs"]
mod tests;

use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};

use chrono::{DateTime, Local};

/// Maximum number of log entries retained in the ring buffer.
const MAX_ENTRIES: usize = 10_000;

/// Minimum level always captured to the ring buffer, regardless of `RUST_LOG`.
const RING_BUFFER_LEVEL: log::LevelFilter = log::LevelFilter::Info;

/// A single captured log entry.
#[derive(Clone, Debug)]
pub struct LogEntry {
    pub timestamp: DateTime<Local>,
    pub level: log::Level,
    pub target: String,
    pub message: String,
}

/// Logger that captures entries into a shared ring buffer and delegates to
/// `env_logger` for stderr output.
struct LogCapture {
    entries: Mutex<VecDeque<LogEntry>>,
    stderr: env_logger::Logger,
}

/// Global logger instance.
static INSTANCE: OnceLock<LogCapture> = OnceLock::new();

/// Initialize the log capture system. Must be called exactly once, before any
/// logging occurs. Replaces the usual `env_logger::init()` call.
pub fn init() {
    let stderr = env_logger::Builder::from_default_env().build();
    let env_level = stderr.filter();

    let capture = INSTANCE.get_or_init(|| LogCapture {
        entries: Mutex::new(VecDeque::with_capacity(MAX_ENTRIES)),
        stderr,
    });

    // Safety: OnceLock guarantees this is a stable reference.
    log::set_logger(capture).expect("logger already set");
    // Accept records needed by either the ring buffer or env_logger.
    log::set_max_level(std::cmp::max(RING_BUFFER_LEVEL, env_level));
}

/// Return a snapshot of all captured log entries.
pub fn entries() -> Vec<LogEntry> {
    INSTANCE
        .get()
        .map(|c| c.entries.lock().unwrap().iter().cloned().collect())
        .unwrap_or_default()
}

/// Return the most recent log entry, if any.
pub fn latest() -> Option<LogEntry> {
    INSTANCE
        .get()
        .and_then(|c| c.entries.lock().unwrap().back().cloned())
}

impl log::Log for LogCapture {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= RING_BUFFER_LEVEL || self.stderr.enabled(metadata)
    }

    fn log(&self, record: &log::Record) {
        // Always capture to ring buffer for levels at or above RING_BUFFER_LEVEL
        if record.level() <= RING_BUFFER_LEVEL {
            let entry = LogEntry {
                timestamp: Local::now(),
                level: record.level(),
                target: record.target().to_string(),
                message: format!("{}", record.args()),
            };
            if let Ok(mut entries) = self.entries.lock() {
                if entries.len() >= MAX_ENTRIES {
                    entries.pop_front();
                }
                entries.push_back(entry);
            }
        }

        // Delegate to stderr based on env_logger filter
        if self.stderr.enabled(record.metadata()) {
            self.stderr.log(record);
        }
    }

    fn flush(&self) {
        self.stderr.flush();
    }
}
