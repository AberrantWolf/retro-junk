//! Notify wrapper: raw backend events in, coalesced [`WatchEvent`]s out.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;
use std::time::Duration;

use notify::Watcher as _;
use retro_junk_archive::WatchBackend;

use super::coalesce::{Coalescer, POLL_INTERVAL};
use super::events::WatchEvent;

#[derive(Debug, thiserror::Error)]
pub enum WatchError {
    #[error(transparent)]
    Notify(#[from] notify::Error),
}

enum Backend {
    Native(notify::RecommendedWatcher),
    Poll(notify::PollWatcher),
}

impl Backend {
    fn watch(&mut self, root: &Path) -> Result<(), notify::Error> {
        match self {
            Self::Native(watcher) => watcher.watch(root, notify::RecursiveMode::Recursive),
            Self::Poll(watcher) => watcher.watch(root, notify::RecursiveMode::Recursive),
        }
    }
}

/// Recursive watcher over a set of roots with hand-rolled debouncing.
pub struct DirectoryWatcher {
    // `Option` so `Drop` can close the raw channel (by dropping the backend)
    // *before* joining the coalescer thread — otherwise the thread never
    // sees `Disconnected` and the join blocks forever.
    backend: Option<Backend>,
    coalesce_thread: Option<JoinHandle<()>>,
}

/// Poll cadence for the fallback backend on network mounts.
const POLL_BACKEND_INTERVAL: Duration = Duration::from_secs(30);

impl DirectoryWatcher {
    /// Watch `roots` recursively, choosing the backend per `preference`:
    /// `Auto` polls when any root sits on a remote mount (native
    /// notification is unreliable over NFS/SMB) and uses the platform
    /// backend otherwise. Returns the watcher plus the settled-event stream.
    pub fn start(
        roots: &[PathBuf],
        preference: WatchBackend,
    ) -> Result<(Self, Receiver<WatchEvent>), WatchError> {
        let (raw_tx, raw_rx) = channel::<notify::Result<notify::Event>>();
        let (out_tx, out_rx) = channel::<WatchEvent>();
        let use_poll = match preference {
            WatchBackend::Native => false,
            WatchBackend::Poll => true,
            WatchBackend::Off | WatchBackend::Auto => roots
                .iter()
                .any(|root| retro_junk_io::remote_mount_kind(root).is_some()),
        };
        let mut backend = if use_poll {
            log::info!("watch: polling backend (remote mount or explicit preference)");
            Backend::Poll(notify::PollWatcher::new(
                raw_tx,
                notify::Config::default().with_poll_interval(POLL_BACKEND_INTERVAL),
            )?)
        } else {
            Backend::Native(notify::RecommendedWatcher::new(
                raw_tx,
                notify::Config::default(),
            )?)
        };
        for root in roots {
            backend.watch(root)?;
        }
        let coalesce_thread = std::thread::Builder::new()
            .name("retro-junk-watch".to_owned())
            .spawn(move || run_coalesce(&raw_rx, &out_tx))
            .expect("spawn watcher thread");
        Ok((
            Self {
                backend: Some(backend),
                coalesce_thread: Some(coalesce_thread),
            },
            out_rx,
        ))
    }
}

impl Drop for DirectoryWatcher {
    fn drop(&mut self) {
        drop(self.backend.take());
        if let Some(handle) = self.coalesce_thread.take() {
            // A panicked coalescer must not abort shutdown.
            let _ = handle.join();
        }
    }
}

fn run_coalesce(raw: &Receiver<notify::Result<notify::Event>>, out: &Sender<WatchEvent>) {
    let mut coalescer = Coalescer::new();
    loop {
        match raw.recv_timeout(POLL_INTERVAL) {
            Ok(Ok(event)) => coalescer.ingest(&event, out),
            Ok(Err(error)) => log::warn!("watch backend error: {error}"),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
        if !coalescer.flush_expired(out) {
            return;
        }
    }
    coalescer.flush_all(out);
}
