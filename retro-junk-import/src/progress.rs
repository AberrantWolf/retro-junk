//! Import progress reporting.

/// Trait for receiving import progress updates.
pub trait ImportProgress {
    /// Called after each game is processed during DAT import.
    fn on_game(&self, current: usize, total: usize, name: &str);

    /// Called when a phase starts (e.g., "Importing Nintendo - NES").
    fn on_phase(&self, message: &str);

    /// Called when the import is complete.
    fn on_complete(&self, message: &str);
}

/// A no-op progress reporter that discards all updates.
pub struct SilentProgress;

impl ImportProgress for SilentProgress {
    fn on_game(&self, _current: usize, _total: usize, _name: &str) {}
    fn on_phase(&self, _message: &str) {}
    fn on_complete(&self, _message: &str) {}
}

/// A progress reporter that logs to the `log` crate.
pub struct LogProgress;

impl ImportProgress for LogProgress {
    fn on_game(&self, current: usize, total: usize, name: &str) {
        if current.is_multiple_of(500) || current == total {
            log::info!("  [{}/{}] {}", current, total, name);
        }
    }

    fn on_phase(&self, message: &str) {
        log::info!("{}", message);
    }

    fn on_complete(&self, message: &str) {
        log::info!("{}", message);
    }
}
