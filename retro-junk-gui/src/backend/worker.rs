use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use retro_junk_io::ProgressUnit;

use crate::app::RetroJunkApp;
use crate::state::{
    AppMessage, BackgroundOperation, OperationKind, OperationOutcome, OperationPhase,
    ProgressDisplay, next_operation_id,
};

/// Build the progress callback that shared operations report through.
///
/// Every long-running operation reports the same three things — which phase it
/// is in, what its numbers count, and how far along it is — and the activity bar
/// wants all three. Building the bridge here means no caller has to decide how a
/// reported unit is rendered, and none of them can get it wrong.
pub(crate) fn forward_phases(
    op_id: u64,
    sender: crate::state::AppMessageSender,
) -> impl Fn(&str, ProgressUnit, u64, u64) {
    move |description, unit, current, total| {
        let _ = sender.send(AppMessage::OperationPhase {
            op_id,
            phase: OperationPhase::reported(description, unit, current, total),
        });
    }
}

/// Deliver an operation-specific result while also returning the lifecycle
/// result consumed by the shared runner. This keeps result payloads and the
/// terminal activity outcome in agreement.
pub(crate) fn deliver_result<T>(
    sender: &crate::state::AppMessageSender,
    result: Result<T, String>,
    message: impl FnOnce(Result<T, String>) -> AppMessage,
) -> Result<(), String> {
    let outcome = result.as_ref().map(|_| ()).map_err(Clone::clone);
    let _ = sender.send(message(result));
    outcome
}

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
    F: FnOnce(u64, Arc<AtomicBool>, crate::state::AppMessageSender) -> Result<(), String>
        + Send
        + 'static,
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
        let terminal_tx = tx.clone();
        let terminal_cancel = cancel.clone();
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(op_id, cancel, tx)));
        let outcome = match result {
            Err(payload) => OperationOutcome::Failed(payload.downcast_ref::<&str>().map_or_else(
                || {
                    payload
                        .downcast_ref::<String>()
                        .cloned()
                        .unwrap_or_else(|| "background worker panicked".to_owned())
                },
                |message| (*message).to_owned(),
            )),
            Ok(_) if terminal_cancel.load(std::sync::atomic::Ordering::Relaxed) => {
                OperationOutcome::Cancelled
            }
            Ok(Ok(())) => OperationOutcome::Succeeded,
            Ok(Err(error)) => OperationOutcome::Failed(error),
        };
        if matches!(outcome, OperationOutcome::Succeeded) {
            terminal_tx.finish_determinate_phase(op_id);
        }
        let _ = terminal_tx.send(AppMessage::OperationComplete { op_id, outcome });
    });
    app.op_threads.insert(op_id, handle);

    op_id
}
