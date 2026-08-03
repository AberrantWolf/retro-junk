//! The backend command surface.
//!
//! Every long-running capability is a plain function here: it takes its
//! inputs, an [`OpCtx`] for progress and cancellation, and returns a typed
//! result. Frontends decide only how to schedule the call (a thread the GUI
//! owns, the current thread for the CLI) and how to render what comes back.
//! No frontend implements an operation itself.

pub mod archive;
pub mod assets;
pub mod catalog_ops;
pub mod chd_compress;
pub mod convergence;
pub mod daemon;
pub mod export;
pub mod fix_cue;
pub mod hash;
pub mod inbox;
pub mod miximage;
pub mod organize;
pub mod playable_build;
pub mod rename;
pub mod scan;
pub mod scrape_media;

use std::sync::atomic::{AtomicBool, Ordering};

use retro_junk_io::PhaseProgressFn;

/// Progress + cancellation handle passed to every backend command.
///
/// `progress` receives (phase description, unit, current, total) — the same
/// shape `retro_junk_io` reports everywhere else. The GUI forwards it to the
/// activity bar; the CLI draws a progress bar; tests pass a no-op.
pub struct OpCtx<'a> {
    pub cancel: &'a AtomicBool,
    pub progress: &'a PhaseProgressFn<'a>,
}

impl<'a> OpCtx<'a> {
    pub fn new(cancel: &'a AtomicBool, progress: &'a PhaseProgressFn<'a>) -> Self {
        Self { cancel, progress }
    }

    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

/// A no-op progress sink for callers that don't render progress.
pub const SILENT_PROGRESS: &PhaseProgressFn<'static> = &|_, _, _, _| {};
