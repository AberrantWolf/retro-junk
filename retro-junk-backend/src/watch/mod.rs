//! Filesystem watching with settle-window coalescing.

pub mod coalesce;
pub mod events;
pub mod watcher;

pub use events::WatchEvent;
pub use watcher::{DirectoryWatcher, WatchError};

#[cfg(test)]
#[path = "../tests/coalesce_tests.rs"]
mod coalesce_tests;
