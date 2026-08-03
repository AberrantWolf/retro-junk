//! The backend: every capability of retro-junk, behind one API.
//!
//! Frontends (CLI, GUI) parse input and render output; this crate does the
//! work. It owns the convergence executor, worker, daemon runtime, automation
//! policy, the serialized projection store ([`store`]), and the command
//! surface ([`ops`]) that both frontends call. No frontend opens a database,
//! walks a filesystem, or decides a status on its own.

pub mod adoption;
pub mod assets;
pub mod completion;
pub mod daemon;
pub mod executor;
pub mod fingerprint;
pub mod incoming;
pub mod library;
pub mod ops;
pub mod policy;
pub mod profiles;
pub mod queries;
pub mod scrape;
pub mod store;
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
