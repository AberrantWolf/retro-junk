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

/// Abstraction over bounded and unbounded mpsc receivers.
///
/// Allows `run_with_events` to work with both `mpsc::Receiver` (bounded,
/// used by the enrich path) and `mpsc::UnboundedReceiver` (used by the
/// scrape path).
#[allow(async_fn_in_trait)]
pub trait EventReceiver<E> {
    /// Receive the next event, returning `None` when the channel is closed.
    async fn recv(&mut self) -> Option<E>;
}

impl<E> EventReceiver<E> for mpsc::Receiver<E> {
    async fn recv(&mut self) -> Option<E> {
        mpsc::Receiver::recv(self).await
    }
}

impl<E> EventReceiver<E> for mpsc::UnboundedReceiver<E> {
    async fn recv(&mut self) -> Option<E> {
        mpsc::UnboundedReceiver::recv(self).await
    }
}

/// Drive an async task while processing events from its channel.
///
/// Runs `task` to completion, calling `on_event` for each event received on
/// `event_rx`. Returns the task's result after the channel is fully drained
/// (or after a timeout if senders are not dropped promptly).
pub async fn run_with_events<F, E, R, Rx>(
    task: F,
    mut event_rx: Rx,
    mut on_event: impl FnMut(E),
) -> R
where
    F: Future<Output = R>,
    Rx: EventReceiver<E> + Unpin,
{
    tokio::pin!(task);
    let mut result = None;
    let mut event_count: u64 = 0;

    log::debug!("run_with_events: starting event loop");

    // Phase 1: select between task completion and events
    loop {
        tokio::select! {
            r = &mut task, if result.is_none() => {
                log::debug!(
                    "run_with_events: task completed ({} events received so far)",
                    event_count,
                );
                result = Some(r);
                // Task done — break to drain phase
                break;
            }
            event = event_rx.recv() => {
                match event {
                    Some(e) => {
                        event_count += 1;
                        on_event(e);
                    }
                    // Channel closed before task finished (unusual but safe)
                    None => {
                        log::debug!(
                            "run_with_events: channel closed before task finished ({} events)",
                            event_count,
                        );
                        break;
                    }
                }
            }
        }
    }

    // Phase 2: drain remaining events with a timeout
    if result.is_some() {
        log::debug!(
            "run_with_events: draining remaining events (timeout: {}s)",
            DRAIN_TIMEOUT.as_secs()
        );
        let deadline = Instant::now() + DRAIN_TIMEOUT;
        let mut drain_count: u64 = 0;
        loop {
            match tokio::time::timeout_at(deadline, event_rx.recv()).await {
                Ok(Some(e)) => {
                    drain_count += 1;
                    on_event(e);
                }
                Ok(None) => {
                    log::debug!(
                        "run_with_events: drain complete ({} drained, {} total events)",
                        drain_count,
                        event_count + drain_count
                    );
                    break;
                }
                Err(_) => {
                    log::warn!(
                        "run_with_events: drain timed out after {}s ({} drained, senders likely leaked)",
                        DRAIN_TIMEOUT.as_secs(),
                        drain_count,
                    );
                    break;
                }
            }
        }
    }

    // If channel closed before task completed, await the task directly
    match result {
        Some(r) => r,
        None => {
            log::debug!("run_with_events: awaiting task (channel closed first)");
            task.await
        }
    }
}
