//! Re-export shim: the fingerprint implementation moved to
//! `retro_junk_backend::fingerprint` (pure domain logic shared by scan
//! operations). Existing `crate::fingerprint::` paths keep working.

pub use retro_junk_backend::fingerprint::*;
