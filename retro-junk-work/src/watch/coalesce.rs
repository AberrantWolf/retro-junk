//! Debounce and rename inference over raw `notify` events.
//!
//! Raw filesystem notification is noisy: a copy-in-progress fires dozens of
//! modify events, and platforms disagree about how renames arrive. The
//! coalescer settles each path behind a debounce window, pairs rename halves
//! by notify tracker id (cross-directory-safe) or basename (Remove→Create
//! within a short window), and keeps "modified" sticky over "added" so a
//! changed file is always re-processed. Extension policy belongs to the
//! consumer — incoming drop folders accept anything, playable roots filter.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use super::events::WatchEvent;

/// Per-path settle window.
pub const DEBOUNCE: Duration = Duration::from_millis(500);
/// How long a removed basename / rename source is retained for pairing.
pub const RENAME_WINDOW: Duration = Duration::from_secs(2);
/// Coalescer wake cadence; the upper bound on quiet-period latency.
pub const POLL_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingKind {
    Added,
    Modified,
}

struct Pending {
    kind: PendingKind,
    deadline: Instant,
}

pub struct Coalescer {
    pending: HashMap<PathBuf, Pending>,
    /// basename → (full path, removal time), for Remove→Create rename pairs.
    recent_removes: HashMap<String, (PathBuf, Instant)>,
    /// notify tracker id → rename source.
    rename_from_by_tracker: HashMap<usize, (PathBuf, Instant)>,
    /// basename fallback when the backend supplies no tracker id.
    rename_from_by_basename: HashMap<String, (PathBuf, Instant)>,
}

impl Coalescer {
    #[must_use]
    pub fn new() -> Self {
        Self {
            pending: HashMap::new(),
            recent_removes: HashMap::new(),
            rename_from_by_tracker: HashMap::new(),
            rename_from_by_basename: HashMap::new(),
        }
    }

    /// Feed one raw notify event; settled events go straight to `out`.
    pub fn ingest(&mut self, event: &notify::Event, out: &Sender<WatchEvent>) {
        use notify::EventKind;
        use notify::event::{ModifyKind, RenameMode};

        match &event.kind {
            EventKind::Modify(ModifyKind::Name(RenameMode::Both)) if event.paths.len() == 2 => {
                let _ = out.send(WatchEvent::FileRenamed {
                    from: event.paths[0].clone(),
                    to: event.paths[1].clone(),
                });
            }
            EventKind::Modify(ModifyKind::Name(RenameMode::From)) => {
                let now = Instant::now();
                for path in &event.paths {
                    if let Some(tracker) = event.attrs.tracker() {
                        self.rename_from_by_tracker
                            .insert(tracker, (path.clone(), now));
                    }
                    if let Some(name) = basename(path) {
                        self.rename_from_by_basename
                            .insert(name, (path.clone(), now));
                    }
                }
            }
            EventKind::Modify(ModifyKind::Name(RenameMode::To)) => {
                for path in &event.paths {
                    if let Some(from) = self.take_rename_source(event.attrs.tracker(), path) {
                        let _ = out.send(WatchEvent::FileRenamed {
                            from,
                            to: path.clone(),
                        });
                    } else {
                        self.upsert(path.clone(), PendingKind::Added);
                    }
                }
            }
            kind => {
                for path in &event.paths {
                    match kind {
                        EventKind::Create(notify::event::CreateKind::Folder) => {
                            let _ = out.send(WatchEvent::DirAdded(path.clone()));
                        }
                        EventKind::Remove(notify::event::RemoveKind::Folder) => {
                            let _ = out.send(WatchEvent::DirRemoved(path.clone()));
                        }
                        EventKind::Create(_) => {
                            if let Some(from) = self.take_recent_remove(path) {
                                let _ = out.send(WatchEvent::FileRenamed {
                                    from,
                                    to: path.clone(),
                                });
                            } else {
                                self.upsert(path.clone(), PendingKind::Added);
                            }
                        }
                        EventKind::Modify(ModifyKind::Data(_) | ModifyKind::Any)
                        | EventKind::Any => {
                            self.upsert(path.clone(), PendingKind::Modified);
                        }
                        EventKind::Remove(_) => {
                            if let Some(name) = basename(path) {
                                self.recent_removes
                                    .insert(name, (path.clone(), Instant::now()));
                            }
                            self.pending.remove(path);
                            let _ = out.send(WatchEvent::FileRemoved(path.clone()));
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    /// Emit every pending event whose settle window elapsed; prune stale
    /// rename state. Returns `false` when the consumer is gone.
    pub fn flush_expired(&mut self, out: &Sender<WatchEvent>) -> bool {
        let now = Instant::now();
        let ready: Vec<PathBuf> = self
            .pending
            .iter()
            .filter(|(_, pending)| pending.deadline <= now)
            .map(|(path, _)| path.clone())
            .collect();
        for path in ready {
            let Some(pending) = self.pending.remove(&path) else {
                continue;
            };
            let event = match pending.kind {
                PendingKind::Added => WatchEvent::FileAdded(path),
                PendingKind::Modified => WatchEvent::FileModified(path),
            };
            if out.send(event).is_err() {
                return false;
            }
        }
        self.recent_removes
            .retain(|_, (_, at)| now.duration_since(*at) < RENAME_WINDOW);
        self.rename_from_by_tracker
            .retain(|_, (_, at)| now.duration_since(*at) < RENAME_WINDOW);
        self.rename_from_by_basename
            .retain(|_, (_, at)| now.duration_since(*at) < RENAME_WINDOW);
        true
    }

    /// Drain everything regardless of deadlines (shutdown).
    pub fn flush_all(&mut self, out: &Sender<WatchEvent>) {
        for (path, pending) in self.pending.drain() {
            let event = match pending.kind {
                PendingKind::Added => WatchEvent::FileAdded(path),
                PendingKind::Modified => WatchEvent::FileModified(path),
            };
            let _ = out.send(event);
        }
    }

    fn upsert(&mut self, path: PathBuf, incoming: PendingKind) {
        let deadline = Instant::now() + DEBOUNCE;
        self.pending
            .entry(path)
            .and_modify(|pending| {
                // Once modified, stay modified: derived state must refresh.
                if incoming == PendingKind::Modified {
                    pending.kind = PendingKind::Modified;
                }
                pending.deadline = deadline;
            })
            .or_insert(Pending {
                kind: incoming,
                deadline,
            });
    }

    fn take_rename_source(
        &mut self,
        tracker: Option<usize>,
        to: &std::path::Path,
    ) -> Option<PathBuf> {
        if let Some(tracker) = tracker
            && let Some((from, at)) = self.rename_from_by_tracker.remove(&tracker)
        {
            if let Some(name) = basename(&from) {
                self.rename_from_by_basename.remove(&name);
            }
            if at.elapsed() < RENAME_WINDOW {
                return Some(from);
            }
        }
        let name = basename(to)?;
        let (from, at) = self.rename_from_by_basename.remove(&name)?;
        (at.elapsed() < RENAME_WINDOW).then_some(from)
    }

    fn take_recent_remove(&mut self, created: &std::path::Path) -> Option<PathBuf> {
        let name = basename(created)?;
        let (from, at) = self.recent_removes.remove(&name)?;
        (at.elapsed() < RENAME_WINDOW).then_some(from)
    }
}

impl Default for Coalescer {
    fn default() -> Self {
        Self::new()
    }
}

fn basename(path: &std::path::Path) -> Option<String> {
    path.file_name()
        .and_then(|value| value.to_str())
        .map(str::to_owned)
}
