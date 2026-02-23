//! Worker pool for concurrent processing with backpressure.
//!
//! Spawns N persistent tokio tasks that pull work items from a bounded
//! async-channel. Results are sent to an unbounded channel for consumption
//! by the caller.
//!
//! Uses `async-channel` for work distribution — its `Receiver` is `Clone`,
//! so each worker gets its own handle with no `Mutex` needed. This avoids
//! the `Arc<Mutex<mpsc::Receiver>>` anti-pattern where one worker holds the
//! lock while blocked on `recv()`, starving all others.

use std::future::Future;
use std::sync::Arc;

use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::Duration;

/// Hard safety-net timeout per work item. If a process_fn hangs beyond this,
/// the worker drops the future and moves on. This prevents total pool deadlock
/// when all other timeout layers somehow fail. Set above application-level
/// timeouts (lookup: 60s, worker item: 90s) so it only fires as a last resort.
const SAFETY_TIMEOUT: Duration = Duration::from_secs(120);

/// A pool of worker tasks that process items concurrently.
///
/// Workers are spawned as persistent tokio tasks that pull from a bounded
/// work channel. This provides:
/// - Natural backpressure when all workers are busy
/// - Clean shutdown by dropping the work sender
/// - Single concurrency control point (worker count)
/// - Safety-net timeout per item to prevent deadlocks
///
/// # Example
///
/// ```ignore
/// let mut pool = WorkerPool::start(4, items, |item| async move {
///     process(item).await
/// });
///
/// while let Some(result) = pool.recv().await {
///     handle(result);
/// }
/// ```
pub struct WorkerPool<R: Send + 'static> {
    result_rx: mpsc::UnboundedReceiver<R>,
    _handles: Vec<JoinHandle<()>>,
}

impl<R: Send + 'static> WorkerPool<R> {
    /// Spawn N workers, submit all items, and return a pool for receiving results.
    ///
    /// Items are submitted via a bounded channel (capacity N) providing
    /// natural backpressure. Each worker pulls items one at a time and
    /// invokes `process_fn`. Results are available via [`recv()`](Self::recv).
    ///
    /// A safety-net timeout is applied per item. If a `process_fn` call
    /// exceeds this, the future is dropped and the worker moves to the next
    /// item. The caller receives one fewer result than expected but the pool
    /// does not deadlock.
    ///
    /// Submission happens in a background task so the caller can start
    /// receiving results immediately without deadlock.
    pub fn start<W, F, Fut>(n: usize, items: Vec<W>, process_fn: F) -> Self
    where
        W: Send + 'static,
        F: Fn(W) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = R> + Send + 'static,
    {
        let (work_tx, work_rx) = async_channel::bounded::<W>(n);
        let (result_tx, result_rx) = mpsc::unbounded_channel::<R>();
        let process_fn = Arc::new(process_fn);

        // Spawn workers — each gets a cloned Receiver (no Mutex needed)
        let handles: Vec<JoinHandle<()>> = (0..n)
            .map(|_| {
                let work_rx = work_rx.clone();
                let result_tx = result_tx.clone();
                let process_fn = process_fn.clone();
                tokio::spawn(async move {
                    while let Ok(item) = work_rx.recv().await {
                        // Process the item with a hard safety timeout.
                        match tokio::time::timeout(SAFETY_TIMEOUT, process_fn(item)).await {
                            Ok(r) => {
                                if result_tx.send(r).is_err() {
                                    break; // Receiver dropped
                                }
                            }
                            Err(_) => {
                                log::debug!(
                                    "Worker pool: item timed out after {}s, skipping",
                                    SAFETY_TIMEOUT.as_secs()
                                );
                                // No result sent — caller gets one fewer result.
                                // Worker continues to next item.
                            }
                        }
                    }
                    // Channel closed (sender dropped) → worker exits
                })
            })
            .collect();

        // Drop our copy of result_tx so the channel closes when all workers finish
        drop(result_tx);

        // Spawn submission task
        tokio::spawn(async move {
            for item in items {
                if work_tx.send(item).await.is_err() {
                    break;
                }
            }
            // work_tx dropped here -> channel closes -> workers drain remaining items then stop
        });

        Self {
            result_rx,
            _handles: handles,
        }
    }

    /// Receive the next result. Returns `None` when all items have been
    /// processed and all workers have shut down.
    pub async fn recv(&mut self) -> Option<R> {
        self.result_rx.recv().await
    }
}
