//! Convergence executor, worker, and automation policy.
//!
//! The daemon, CLI `sync`, and GUI queue actions are three callers of one
//! derivation (`retro_junk_db::convergence`) and one executor
//! ([`executor::execute_action`]). This crate adds coordination — claims,
//! locking etiquette, stage ordering, policy gates — and no archive logic;
//! that lives in `retro_junk_lib::archive_ops` and below.

pub mod adoption;
pub mod daemon;
pub mod executor;
pub mod incoming;
pub mod policy;
pub mod profiles;
pub mod scrape;
pub mod suggestions;
pub mod watch;
pub mod worker;

pub use adoption::{
    ADOPT_SUGGESTION_KIND, AdoptionCandidate, AdoptionCandidateKind, AdoptionSuggestionPayload,
    ignore_playables, ignore_rules, open_adoption_suggestion, unignore_playables,
};
pub use executor::{
    ActionOutcome, ExecContext, ForceRebuildOutcome, LockEtiquette, ReconcileMode, ScrapeSettings,
    ToolPaths, WorkError, execute_action, force_rebuild_playable,
};
pub use policy::{AutoImportMode, AutomationPolicy, BindConfidence};
pub use suggestions::{
    OfferedActions, SCRAPE_SUGGESTION_KIND, ScrapeSuggestionPayload, apply_suggestion,
    apply_suggestion_choice, is_applicable, offered_actions,
};
pub use watch::{DirectoryWatcher, WatchEvent};
pub use worker::{ProjectionPass, RunMode, RunStats, run_once};
