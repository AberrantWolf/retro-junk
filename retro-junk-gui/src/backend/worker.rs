use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crate::app::RetroJunkApp;
use crate::state::{BackgroundOperation, OperationKind, ProgressDisplay, next_operation_id};

/// Spawn a background operation with the standard boilerplate:
/// allocates an operation ID, creates a cancellation token, registers
/// the operation on `app.operations`, clones the message sender, and
/// spawns a thread that runs the provided closure. The `JoinHandle` is
/// tracked on `app.op_threads` so `on_exit` can join it before the process
/// dies mid-write (D2).
///
/// The closure receives `(op_id, cancel_token, message_sender)`.
/// Returns the allocated operation ID.
pub fn spawn_background_op<F>(
    app: &mut RetroJunkApp,
    description: String,
    kind: OperationKind,
    scope: String,
    display: ProgressDisplay,
    work: F,
) -> u64
where
    F: FnOnce(u64, Arc<AtomicBool>, crate::state::AppMessageSender) + Send + 'static,
{
    let op_id = next_operation_id();
    let cancel = Arc::new(AtomicBool::new(false));
    let tx = app.message_tx.clone();

    app.operations.push(BackgroundOperation::new(
        op_id,
        description,
        cancel.clone(),
        kind,
        scope,
        display,
    ));

    let handle = std::thread::spawn(move || {
        work(op_id, cancel, tx);
    });
    app.op_threads.insert(op_id, handle);

    op_id
}
