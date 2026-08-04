//! NEC console analyzers.
//!
//! Covers the 1987 PC Engine and the console NEC sold as the same hardware
//! elsewhere — the TurboGrafx-16 in North America — plus the CD-ROM² add-on
//! (TurboGrafx-CD / Turbo Duo in North America).
//!
//! The card-based and disc-based sides are separate analyzers because they
//! read separate media and match against separate databases: `HuCard`s against
//! No-Intro, discs against Redump.

pub mod pce;
pub mod pce_cd;

pub use pce::PceAnalyzer;
pub use pce_cd::PceCdAnalyzer;
