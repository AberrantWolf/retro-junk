//! Convergence executor, worker, and automation policy.
//!
//! The daemon, CLI `sync`, and GUI queue actions are three callers of one
//! derivation (`retro_junk_db::convergence`) and one executor
//! ([`executor::execute_action`]). This crate adds coordination — claims,
//! locking etiquette, stage ordering, policy gates — and no archive logic;
//! that lives in `retro_junk_lib::archive_ops` and below.

pub mod daemon;
pub mod executor;
pub mod incoming;
pub mod policy;
pub mod profiles;
pub mod watch;
pub mod worker;

pub use executor::{
    ActionOutcome, ExecContext, LockEtiquette, ReconcileMode, ToolPaths, WorkError, execute_action,
};
pub use policy::{AutoImportMode, AutomationPolicy, BindConfidence};
pub use watch::{DirectoryWatcher, WatchEvent};
pub use worker::{ProjectionPass, RunMode, RunStats, run_once};
