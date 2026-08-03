//! Automation policy: which convergence actions run unattended.
//!
//! Defaults are automation-first for safe, idempotent work — leave the tool
//! running and the archive converges to a fully verified, fully playable
//! set. Walls remain only where actions touch files outside the tool's
//! ownership (imports) or destroy anything (never automated at all).
//!
//! Persisted as the `[automation]` table of the shared
//! `~/.config/retro-junk/settings.toml`, edited by GUI Settings and
//! hot-reloaded by the daemon on file mtime change. Every field ships with
//! consuming logic — no dead config.

use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// How the daemon treats identified incoming packages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutoImportMode {
    /// Import unambiguous, confidently identified packages automatically.
    On,
    /// Pre-process fully, then file a suggestion for one-click review.
    #[default]
    Suggest,
    /// Track packages but neither plan-apply nor suggest.
    Off,
}

/// Identification strength required before an import may auto-execute.
/// Ordered weakest to strongest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BindConfidence {
    /// A serial recognized from the containing folder's name.
    FolderSerial,
    /// A serial read from the dump's own header.
    ExactSerial,
    /// Complete content hashes matched one catalog medium.
    #[default]
    ExactHash,
}

// Independent on/off switches over unrelated behaviors, each one a line in
// settings.toml. Folding them into a state machine would invent states that
// do not exist and make the file less readable, not more.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AutomationPolicy {
    /// Append integrity/catalog verification evidence unattended.
    /// Evidence is append-only; there is nothing to lose.
    pub auto_verify: bool,
    /// Build preferred playable representations, project artwork, and
    /// upsert gamelist entries unattended. One intent — keep the playable
    /// projections current — so one switch.
    pub auto_build: bool,
    /// What to do with identified packages in watched incoming folders.
    pub auto_import: AutoImportMode,
    /// Minimum identification strength for `auto_import = on`.
    pub auto_bind_min_confidence: BindConfidence,
    /// Fetch missing artwork from `ScreenScraper` unattended.
    ///
    /// Off by default, unlike the other automation switches. Those do local,
    /// idempotent work with nothing to lose; this one spends a metered
    /// external quota and adds durable files to the archive. Opt in.
    pub auto_scrape: bool,
    /// Refuse to publish a filename-tier match unattended, filing it for
    /// review instead. A filename match is a guess, and the archive is where
    /// guesses become permanent.
    pub scrape_only_when_unambiguous: bool,
    /// Asset types a release is expected to hold. Also what the artwork
    /// evidence badge counts against, so "complete" means the same thing
    /// everywhere.
    pub scrape_asset_types: Vec<String>,
    /// Stop scraping when the daily request budget falls to this many
    /// remaining, leaving headroom for interactive use. `0` spends it all.
    pub scrape_daily_request_reserve: u32,
    /// Re-read published archive bytes immediately after import instead of
    /// deferring the read-back to background integrity verification.
    /// Doubles network transfer per package; off by default.
    pub verify_published_bytes: bool,
    /// Hours between full archive deep-scans that catch out-of-band edits.
    /// `0` disables the periodic deep scan.
    pub deep_rescan_hours: u32,
}

impl Default for AutomationPolicy {
    fn default() -> Self {
        Self {
            auto_verify: true,
            auto_build: true,
            auto_import: AutoImportMode::Suggest,
            auto_bind_min_confidence: BindConfidence::ExactHash,
            auto_scrape: false,
            scrape_only_when_unambiguous: true,
            scrape_asset_types: default_scrape_asset_types(),
            scrape_daily_request_reserve: 0,
            verify_published_bytes: false,
            deep_rescan_hours: 24,
        }
    }
}

/// The artwork set a release is expected to hold out of the box — the
/// scraper's own default, named here so the settings file is self-describing.
fn default_scrape_asset_types() -> Vec<String> {
    retro_junk_frontend::AssetSelection::default().names()
}

impl AutomationPolicy {
    /// The expected artwork set, as the rest of the workspace speaks it.
    ///
    /// An empty or unrecognized list falls back to the default rather than
    /// meaning "expect nothing", which would silently report every release as
    /// fully scraped.
    #[must_use]
    pub fn scrape_selection(&self) -> retro_junk_frontend::AssetSelection {
        let selection = retro_junk_frontend::AssetSelection::from_names(&self.scrape_asset_types);
        if selection.types.is_empty() {
            retro_junk_frontend::AssetSelection::default()
        } else {
            selection
        }
    }

    /// Everything the executor needs to run a scrape, resolved from policy.
    #[must_use]
    pub fn scrape_settings(&self) -> crate::executor::ScrapeSettings {
        crate::executor::ScrapeSettings {
            expected_assets: self.scrape_selection(),
            only_when_unambiguous: self.scrape_only_when_unambiguous,
            daily_request_reserve: self.scrape_daily_request_reserve,
        }
    }
    /// Load from the shared settings file. Never errors: a missing file,
    /// missing table, or unparsable field yields the defaults — policy
    /// must not be able to wedge startup.
    #[must_use]
    pub fn load() -> Self {
        Self::load_from(&retro_junk_lib::settings::settings_path())
    }

    #[must_use]
    pub fn load_from(path: &Path) -> Self {
        let Ok(contents) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        let Ok(doc) = contents.parse::<toml::Value>() else {
            return Self::default();
        };
        doc.get("automation")
            .cloned()
            .and_then(|value| value.try_into().ok())
            .unwrap_or_default()
    }

    /// Persist as the `[automation]` table, surgically — every other key in
    /// settings.toml is preserved byte-for-meaning.
    pub fn save(&self) -> io::Result<()> {
        self.save_to(&retro_junk_lib::settings::settings_path())
    }

    pub fn save_to(&self, path: &Path) -> io::Result<()> {
        let mut doc: toml::Value = std::fs::read_to_string(path)
            .ok()
            .and_then(|contents| contents.parse().ok())
            .unwrap_or_else(|| toml::Value::Table(toml::map::Map::default()));
        let table = doc
            .as_table_mut()
            .ok_or_else(|| io::Error::other("settings.toml root is not a table"))?;
        let automation = toml::Value::try_from(self).map_err(io::Error::other)?;
        table.insert("automation".to_owned(), automation);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let serialized = toml::to_string_pretty(&doc).map_err(io::Error::other)?;
        let temporary = path.with_extension("toml.tmp");
        std::fs::write(&temporary, &serialized)?;
        std::fs::rename(&temporary, path)?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "tests/policy_tests.rs"]
mod tests;
