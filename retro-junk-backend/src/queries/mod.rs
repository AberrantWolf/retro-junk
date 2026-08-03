//! The backend query surface.
//!
//! Read-only questions, answered from the projection and the catalog, with
//! all status semantics applied here (via [`crate::completion`]) so every
//! frontend renders the same answer. No frontend runs SQL or folds facts.

pub mod releases;
