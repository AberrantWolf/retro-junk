//! Thin re-export of `retro_junk_backend::ops::daemon`. Process control,
//! status, and log tailing all live in the backend; the GUI only renders
//! what they return.

pub use retro_junk_backend::ops::daemon::{DaemonStatus, log_tail, start, status, stop};
