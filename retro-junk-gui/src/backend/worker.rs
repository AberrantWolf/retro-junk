use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;

use crate::app::RetroJunkApp;
use crate::state::{AppMessage, BackgroundOperation, next_operation_id};

/// Spawn a background operation with the standard boilerplate:
/// allocates an operation ID, creates a cancellation token, registers
/// the operation on `app.operations`, clones the message sender, and
/// spawns a thread that runs the provided closure.
///
/// The closure receives `(op_id, cancel_token, message_sender)`.
/// Returns the allocated operation ID.
pub fn spawn_background_op<F>(app: &mut RetroJunkApp, description: String, work: F) -> u64
where
    F: FnOnce(u64, Arc<AtomicBool>, mpsc::Sender<AppMessage>) + Send + 'static,
{
    let op_id = next_operation_id();
    let cancel = Arc::new(AtomicBool::new(false));
    let tx = app.message_tx.clone();

    app.operations
        .push(BackgroundOperation::new(op_id, description, cancel.clone()));

    std::thread::spawn(move || {
        work(op_id, cancel, tx);
    });

    op_id
}
