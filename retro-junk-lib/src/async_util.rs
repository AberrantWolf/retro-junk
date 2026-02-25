//! Async utilities for driving tasks with event channels.
//!
//! Provides a reusable pattern for running an async operation while draining
//! its event channel. Used by CLI, GUI, and web frontends to process events
//! (progress updates, results) from library-level async operations.

use std::future::Future;

use tokio::sync::mpsc;
use tokio::time::{Duration, Instant};

/// Maximum time to drain remaining events after the task completes.
/// Defense-in-depth: if senders are leaked (detached tasks holding clones),
/// we don't block forever waiting for the channel to close.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

/// Drive an async task while processing events from its channel.
///
/// Runs `task` to completion, calling `on_event` for each event received on
/// `event_rx`. Returns the task's result after the channel is fully drained
/// (or after a timeout if senders are not dropped promptly).
pub async fn run_with_events<F, E, R>(
    task: F,
    mut event_rx: mpsc::UnboundedReceiver<E>,
    mut on_event: impl FnMut(E),
) -> R
where
    F: Future<Output = R>,
{
    tokio::pin!(task);
    let mut result = None;

    // Phase 1: select between task completion and events
    loop {
        tokio::select! {
            r = &mut task, if result.is_none() => {
                result = Some(r);
                // Task done — break to drain phase
                break;
            }
            event = event_rx.recv() => {
                match event {
                    Some(e) => on_event(e),
                    // Channel closed before task finished (unusual but safe)
                    None => break,
                }
            }
        }
    }

    // Phase 2: drain remaining events with a timeout
    if result.is_some() {
        let deadline = Instant::now() + DRAIN_TIMEOUT;
        loop {
            match tokio::time::timeout_at(deadline, event_rx.recv()).await {
                Ok(Some(e)) => on_event(e),
                Ok(None) => break,    // channel closed cleanly
                Err(_) => break,      // drain timeout — senders likely leaked
            }
        }
    }

    // If channel closed before task completed, await the task directly
    match result {
        Some(r) => r,
        None => task.await,
    }
}
