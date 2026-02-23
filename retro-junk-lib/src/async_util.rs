//! Async utilities for driving tasks with event channels.
//!
//! Provides a reusable pattern for running an async operation while draining
//! its event channel. Used by CLI, GUI, and web frontends to process events
//! (progress updates, results) from library-level async operations.

use std::future::Future;

use tokio::sync::mpsc;

/// Drive an async task while processing events from its channel.
///
/// Runs `task` to completion, calling `on_event` for each event received on
/// `event_rx`. Returns the task's result after the channel is fully drained.
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
    loop {
        tokio::select! {
            r = &mut task, if result.is_none() => { result = Some(r); }
            event = event_rx.recv() => {
                match event {
                    Some(e) => on_event(e),
                    None => break,
                }
            }
        }
    }
    result.unwrap()
}
